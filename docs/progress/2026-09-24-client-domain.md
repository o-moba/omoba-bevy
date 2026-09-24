# 2026-09-24 — Client domain module and render gates (roadmap step 10, slices 10a + 10d)

## Goal
Step 10 starts with two small, low-risk slices from
[plans/client-10-15.md](../plans/client-10-15.md):
- 10a: give the model types that `net`, gameplay and presentation share one
  home, so `net` no longer depends on `combat`, `player` and `team` just for
  them, and add the `RoundId` type that step 15 builds on.
- 10d: register the 2D and 3D presentation backends behind run conditions
  instead of relying only on the early `return` inside each system.

No behaviour change and no wire change. `shared/` and `server/` are untouched.

## 10a: `client/src/domain/`
| File | Contents |
| --- | --- |
| `domain/mod.rs` | module list and re-exports |
| `domain/round.rs` | new `RoundId { server_epoch, match_id }`, `RoundId::from_meta(&SnapshotMeta) -> Option<RoundId>` (`None` when either part is 0), two unit tests |
| `domain/team.rs` | `Team` (derives and serde attribute unchanged), `From` both ways with `shared::map::Team`, the two `PartialEq` bridges, `as_str` |
| `domain/stats.rs` | `CombatStats` (+ `Default`, `is_alive`) and the `MAX_HP`/`MAX_MANA` re-export of `shared::hero_balance` it uses |
| `domain/actors.rs` | `Player`, `PlayerBody`, `VerticalVelocity`, `RemotePlayer`, `MovementTarget`, `MovementRoute` |

- Old paths re-export the moved items: `team.rs` (`pub use crate::domain::Team`),
  `combat.rs` (`pub use crate::domain::{CombatStats, MAX_HP}`), `player.rs`
  (`pub use` for the markers, `pub(crate) use` for the movement intents),
  `net/components.rs` (`pub use crate::domain::RemotePlayer`, which the
  `net::components::*` glob re-exports as `crate::net::RemotePlayer`). No
  other file changed its imports.
- `Team::ui_color`/`ui_hover_color` stay in `team.rs` as a second
  `impl Team` block next to the only UI that uses them. The domain module
  holds no presentation colours.
- `MAX_MANA` is no longer re-exported from `combat`, because nothing used
  that path and the re-export triggered the `unused_imports` lint. It stays
  in `domain/stats.rs`.
- `CombatPointerInputSet` and `WorldMovementInputSet` moved from `combat.rs`
  to `input_context.rs` (verbatim). `combat.rs` re-exports them, so
  `player.rs`, `minimap.rs` and `mobile_controls.rs` are unchanged.
- The two `(server_epoch, match_id)` helpers now hold `Option<RoundId>`
  (the only change is the type):
  - `combat.rs` `CombatRoundIdentity` / `reset_round_input_state`: `let Some(identity) = RoundId::from_meta(..) else { return }`.
  - `mobile_controls.rs` `MobileControls.round_identity`: its test fixture now writes
    `Some(RoundId { server_epoch: 1, match_id: 1 })`.

  The other round detectors (shop, edge HUD, sandbox, draft, presentation3d
  and the event cursors) still use tuples. Switching the two helpers above
  to `SessionEvent::RoundChanged` is slice 15c.

## 10d: backend gates
`sprite.rs` next to `PlayerVisualMode`:
`pub(crate) fn in_models3d()` and `in_sprite2d()`, both
`resource_exists_and_equals(PlayerVisualMode::…)`.

Gated with `in_sprite2d()`:
- `sprite.rs` `SpriteVisualsPlugin`: Startup `load_sprite_visual_assets`; Update chain `reconcile_sprite_identity → attach_sprite_visuals → animate_sprite_visuals`
- `presentation2d.rs` `Presentation2dPlugin`: Startup `load_presentation_assets`; Update `emit_combat_effects → animate_effects`; the PostUpdate attach/sync chain (9 systems)
- `world2d.rs` `World2dPlugin`: Startup `load_world2d_assets → setup_world2d`; Update `apply_prop_profiles`

Gated with `in_models3d()`:
- `presentation3d.rs`: PostUpdate `collect_feedback → draw_feedback`
- `verdant3d.rs`: Startup `load_assets → spawn_environment`; PostUpdate `reconcile_structures`
- `jungle.rs`: Update `attach_jungle_creatures`; PostUpdate `ground_jungle_creatures`
- `minions.rs`: Startup `setup_creature_assets`; Update `attach_minion_models → update_minion_attack_pulses → animate_creatures`
- `bosses.rs`: Startup `load_boss_assets`; the Update set (attach, animation library, bind, sync, double-sided, nameplates)
- `decor.rs`: Update `toggle_decor_visibility`
- `map_visuals.rs`: the PostUpdate prop chain `import_authored_props → initialize_props → reconcile_props → tint_props` (`request_config`/`apply_config` stay ungated because `world2d` reads the registry)
- `map_visuals/river.rs`: the `repair_river → remove_orphaned_replacements` registration
- `projectile_visuals.rs`: Startup `setup_assets`; PostUpdate `attach_visuals → update_visuals → animate_orbits` (`draw_trails` stays ungated)
- `world.rs` `SetupPlugin`: `sync_selected_player_assets`, `force_vrm_models_double_sided`, `apply_lighting_settings_system` (one set)

Not gated:
- `battlefield_atmosphere.rs`. The plan asked for this check first. Its
  `sync_visibility` has no mode guard, and the mist is a screen-space UI
  node that is shown in both modes while the game is Running. Gating it
  would remove the mist from 2D. Its test
  (`overlay_is_single_noninteractive_full_viewport_and_match_scoped`) also
  adds the plugin without a `PlayerVisualMode`.
- The plan's "must run in both modes" list: camera, snapshot apply, combat
  bars, targeting draw, VFX, combat feedback, supporter auras, minimap, team
  vision, `setup_scene`/`setup_main_camera`/fallback spawn, player movement,
  projectile trails, map-visual config, and `register_hero_animation_systems`.

Per-system checks: every gated system either has its own
`if *mode != …` guard, or only touches components that a guarded system
of the same backend spawns (for example `PresentationActorVisual`,
`PlayerSpriteVisual`, `World2dMapProp`, `CreaturePart`, `ProjectileVisual`,
`RiverReplacement`, boss nameplates). In the other mode they were already
no-ops. `decor.rs` reads the mode as an `Option` (a missing mode counts as
3D). Its test registers the system directly, not through the plugin, so the
gate does not affect it. No test that goes through a gated registration
leaves out `PlayerVisualMode`. Every internal guard stays, because tests
register systems directly and flip modes.

Only unused resources differ: in 2D the client no longer creates
`ProjectileAssets` (three meshes plus the projectile materials) or the empty
`BossAssetCache`. Nothing outside the gated systems reads either.

## Tests
- Client lib: 543 → 549.
  - `domain::round::tests::zero_in_either_part_has_no_round`
  - `domain::round::tests::tick_does_not_change_the_round_but_epoch_and_match_do`
  - `sprite::tests::backend_run_conditions_follow_the_mode_and_skip_a_missing_one`
  - `sprite::tests::models3d_runs_no_sprite2d_backend_system`: Models3d app with the three 2D plugins
  - `sprite::tests::sprite2d_runs_no_models3d_backend_system`: Sprite2d app with presentation3d, Verdant, jungle, minions, bosses, decor
  - `sprite::tests::missing_visual_mode_runs_neither_backend_and_does_not_panic`: both sets, no mode

  The backend apps are bare, with no AssetServer, mesh assets, Time or
  MapLayout. A gated system that ran would fail parameter validation and
  panic, and the tests also assert that no entity was spawned. A check with
  both conditions forced to `true` made all four tests fail. The existing
  opposite-mode tests still pass through the gated plugins:
  `jungle::tests::sprite_jungle_keeps_authoritative_roots_without_allocating_3d_assets`,
  `minions::tests::sprite2d_minions_do_not_allocate_or_attach_procedural_3d_assets`
  and `verdant3d::tests::sprite2d_never_loads_or_spawns_the_verdant_scene`.
- Shared: 78, unchanged.

## Checks
- `cargo fmt --all -- --check` clean.
- `cargo clippy --workspace --all-targets --no-deps -- -D warnings` clean.
- `cargo test -p client --lib`: 549 passed. `cargo test -p shared`: 78 passed.

## Notes for the next slices
- 10b/10c can now move `combat.rs`/`player.rs` without carrying the domain
  types. `CombatStats` and the markers are already out.
- 10e (plugin groups) should keep `battlefield_atmosphere` in the shared
  presentation group, not the 3D one, because the mist runs in both modes.
- 10i (optional) can switch callers from the re-export shims to
  `crate::domain::…`.
