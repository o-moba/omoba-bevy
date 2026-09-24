# 2026-09-24 — Client debug module and session accessors (roadmap slices 11d + 15e)

## Goal
Two client slices in one PR:
- 11d from [plans/steps-11-13.md](../plans/steps-11-13.md) (Step 11, 11.2
  "Client"): one client copy of the debug toggles, one debug network command,
  the debug files under `client/src/debug/`, and the `DebugPlugins` group.
- 15e from [plans/client-10-15.md](../plans/client-10-15.md) (15.5): the
  `ClientSession` fields become private to `net`, with accessors for outside
  readers and `#[cfg(test)]` setters for the test fixtures. This completes
  step 15.

No wire change. Only `client/` and the docs changed.

## 11d: `client/src/debug/`
| Old | New |
| --- | --- |
| `client/src/god_mode.rs` | `client/src/debug/hud.rs` (`GodModePlugin`: F2/F3, the two buttons, their labels) |
| `client/src/debug_console.rs` | `client/src/debug/console.rs` (unchanged) |
| `client/src/practice_sandbox.rs` | `client/src/debug/tools_page.rs` (`PracticeSandboxPlugin`) |
| (new) | `client/src/debug/mod.rs`: `DebugToggles`, `resend_debug_toggles`, re-exports |

Entity `Name`s (`GodModeButton`, `SpeedBoostButton`, `DebugConsole`,
`PauseMenuPractice*`) are unchanged. The callers import
`crate::debug::DebugConsole` and `crate::debug::tools_page::*`; nothing is
re-exported at the old paths.

### How the three state copies map onto `DebugToggles`
| Old copy | Writers | Readers | Now |
| --- | --- | --- | --- |
| `god_mode::DebugToggleState.god_mode` | F2, the HUD button, the practice page's toggle | HUD label, 0.5 s re-send | `DebugToggles::god_mode` |
| `PracticeSandboxState.god_mode` | the practice page's toggle; cleared when practice access is lost | the practice page's label | `DebugToggles::god_mode` (the field is removed) |
| `player::DebugSpeedBoost(bool)` | F3, the HUD button | HUD label, re-send, local movement (`input.rs`, `motion.rs`), `mirror_debug_flags_to_network_state` → `NetworkState.speed_boost_active` → the snap threshold | `DebugToggles::speed_boost`; the snapshot stage reads it directly |

- The practice page's reset is kept: `sync_practice_availability` clears
  `god_mode` when the match stops being a practice match. It fires on the
  edge (a `Local<bool>` remembers the last frame), because the old
  every-frame check only ever saw the page's own copy, which could not be
  true outside practice; with one shared flag an every-frame check would
  wipe a HUD toggle in a dev match. Pinned by
  `debug::tools_page::tests::leaving_practice_clears_god_mode_but_a_dev_toggle_survives`.
- Visible difference, only with `OMOBA_DEBUG_UI`: the page's line now shows
  a god mode turned on with F2 or the HUD button, and leaving practice also
  clears the HUD's god mode, so the re-send no longer carries it into the
  next match. Without the env var there is no HUD and no re-send, and the
  page behaves exactly as before.
- `mirror_debug_flags_to_network_state` (plan hazard 6) is gone:
  `apply_snapshot_local_player` takes `Res<DebugToggles>` as its 16th
  parameter. The mirror ran unordered in `Update`, so the stage could see
  the value one frame late; it now sees the current one.
- `DebugToggles` is initialised by `PlayerPlugin` (local movement needs it
  without the debug plugins, as `DebugSpeedBoost` was), `GodModePlugin` and
  `PracticeSandboxPlugin`, and by the two net test fixtures that run the
  snapshot stages.

### One command, one re-send
- `NetworkCommand::Debug(DebugCommand)` replaces `SetGodMode { enabled }`,
  `SetSpeedBoost { enabled }` and `Practice { command }`. The arm in
  `send_network_commands` sends `command.to_packet()` behind
  `join_confirmed()`, exactly as the three arms did.
- `debug::resend_debug_toggles` is the old
  `periodically_assert_debug_toggles`: every 0.5 s while connected, god mode
  then speed boost (`DebugToggles::commands()`). It stays in `GodModePlugin`'s
  chain under `debug_controls_enabled` (`OMOBA_DEBUG_UI`, not in Combat
  Test); widening it is 11e.
- The practice page shows when `DebugAccess::for_match_mode(match_mode)
  .practice` and the join is confirmed, the same set as the old
  `"practice" | "offline_practice"` match.

### Offline
`Simulation::command` first tries `DebugCommand::from_packet` and hands the
command to `Simulation::debug`: god mode as before (local flag, refill, clear
the respawn), practice as before (`Simulation::practice`), and the speed boost
an explicit no-op with a comment (it used to fall into `_ => {}`; offline
accepts any finite client transform, so the boosted local movement works
without a clamp to raise).

### `DebugPlugins`
`plugins.rs` gains `DebugPlugins` (`SandboxPlugin`, `PracticeSandboxPlugin`,
`GodModePlugin`, `DebugConsolePlugin`); `GameplayPlugins` ends with
`CombatPlugin`. `main` adds `DebugPlugins` right after `GameplayPlugins`, so
the build order of every plugin is unchanged.

## 15e: `ClientSession` accessors
All fields are `pub(in crate::net)`. Outside readers moved to:

| Old field read | Accessor | Callers |
| --- | --- | --- |
| `state` | `state()` | `frontend/{mod,home,searching}.rs`, `pause_menu.rs`, `team.rs`, `qa/{visual,social}_qa.rs` |
| `join_flow_committed` | `join_in_flight()` | `career.rs`, `game_state.rs`, `mobile_ui.rs`, `team.rs`, `qa/frontend_flow_qa.rs` |
| `server_addr_display` | `server_addr()` | `match_service.rs`, `career_identity.rs`, `mobile_ui.rs`, `qa/offline_qa.rs` |
| `last_join.as_ref().is_some_and(\|j\| j.prematch)` | `joined_prematch()` | `world.rs` |
| `last_join.is_none()` | `!has_committed_join()` (existing) | `mobile_ui.rs` |

Existing accessors are unchanged: `is_connected()`, `is_offline()`,
`join_confirmed()`, `has_committed_join()`, `join_blocked()`,
`join_rejection()`, `is_choosing_loadout()`, and `abandon_join()` stays the
only public write. There is no `last_join()`: nothing outside `net` needs the
loadout itself, and an unused accessor would fail the `dead_code` lint.

Test-only setters (`#[cfg(test)]`): `set_state_for_test`,
`set_join_in_flight_for_test`, `set_server_addr_for_test`,
`set_joined_prematch_for_test`, `clear_last_join_for_test`, next to the
existing `admitted_for_test`, `reconnecting_for_test`, `queued_for_test` and
`reject_for_test`. They replace the field pokes in `career.rs`,
`career_identity.rs`, `frontend/{mod,home,searching}.rs`, `game_state.rs`,
`help_overlay.rs`, `pause_menu.rs`, `player/tests.rs` and `world.rs`.

## Tests and gate
- Client lib 561 → 562 (541 → 542 with `--no-default-features`): the new
  practice-reset test. The HUD test and the practice-page test now assert
  on `DebugToggles` and `NetworkCommand::Debug`. Shared unchanged.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --no-deps -- -D warnings`, `cargo clippy -p client --lib --no-deps
  --no-default-features -- -D warnings`, `cargo test -p client --lib`
  (with and without default features) and `cargo test -p shared` pass.

## Next
- 11e (owner decision): one tools page driven by `DebugAccess` wherever the
  toggles are allowed, so dev gets the page without `OMOBA_DEBUG_UI` and
  practice gets the re-send.
- 11f, 15f, 15g: optional.
