# 2026-09-25 — Server-driven debug access (roadmap 11e + 11f)

Last slices of step 11 ([plan](../plans/steps-11-13.md), "Step 11"; tracker
row in [REFACTORING.md](../REFACTORING.md)). 11e is a behaviour change the
maintainer approved on 2026-09-25; 11f is wire-visible and additive.

## 11f: `Snapshot.debug_access` (shared, server)

- `shared::debug::DebugAccess` derives `Serialize`/`Deserialize`; both
  fields are `#[serde(default)]` and unknown fields are ignored, so a flag
  added later is additive and a missing one means "not allowed". Encoding:
  `{"toggles":true,"practice":false}`.
- `ServerPacket::Snapshot` gains `debug_access: Option<DebugAccess>` right
  after `sandbox`, with `#[serde(default, skip_serializing_if =
  "Option::is_none")]`. `None` is omitted, not `null`, so `GOLDEN_SNAPSHOT`
  and every snapshot without the field are byte-identical. No
  `PROTOCOL_VERSION` bump; `wire_enums.rs` is unchanged (a struct, no new
  enum).
- Server (`server/src/debug/mod.rs`, `snapshot_debug_access`): the world
  snapshot loop computes `debug_access()` once per broadcast and gives it to
  each recipient with `player.joined`. Decision on who gets it:
  - joined players get `Some`, also when both flags are false (release, and
    a worker-allocated round). The all-false value reveals nothing (no debug
    command works there) and is what lets a new client hide the page in a
    worker round, whose `match_mode` reads `"practice"`;
  - recipients that are only career-authenticated or whose join was
    rejected get `None`, as do the prejoin status reply and the lobby
    snapshot (their constructors set `None`). Cost: about 45 bytes per
    snapshot to a joined player.
- The offline simulation's snapshots carry
  `Some(DebugAccess::for_match_mode(OFFLINE_PRACTICE_MODE))`.

## 11e: client follows access

- `client/src/debug/mod.rs`: `ClientDebugAccess { server, combat_test }`
  with `toggles()` (server toggles and not Combat Test), `practice()` and
  `any()`. `sync_debug_access` (new `DebugAccessPlugin`, `DebugAccessSet`)
  recomputes it every frame: nothing before `join_confirmed()`, then
  `snapshot.debug_access`, else `DebugAccess::for_match_mode(match_mode)`
  (older servers). `combat_test` is `sandbox::requested()` or
  `snapshot.sandbox.is_some()`.
- `DebugToggles` is held at "both off" whenever `toggles()` is false. This
  replaces the practice page's edge reset of god mode only; now the speed
  boost resets too, and it also happens on joining a worker round, a release
  match or a disconnect (a transient `join_confirmed()` drop during a
  reconnect also resets them; the server then gets "off" from the re-send,
  so client and server agree).
- `resend_debug_toggles` moved from the HUD chain into `DebugAccessPlugin`
  and runs wherever `toggles()` is true, without `OMOBA_DEBUG_UI`.
- Tools page (`tools_page.rs`): entry and title "Debug tools" (entity
  `Name`s keep `PauseMenuPractice*`). Three parts, each shown by access:
  toggles (god mode and a new speed boost line) where `toggles()`, bots and
  1v1 where `practice()`, and in Combat Test a note plus a "Combat Test
  panel (F6)" button that sets `SandboxClient.open` and closes the menu.
  The entry shows when any part applies. Every action is checked against
  its own access, not just "in practice".
- HUD (`hud.rs`): the buttons still spawn only with `OMOBA_DEBUG_UI` and
  outside Combat Test, now hidden until `toggles()` allows them; F2/F3 and
  the buttons react only then. `OMOBA_DEBUG_UI` still enables: the HUD
  buttons and F2/F3, the on-screen log (`console.rs`) and F8 debug flight.

## Behaviour by mode

| | dev | dev + Combat Test | practice (local) | practice (worker) | offline | release |
|---|---|---|---|---|---|---|
| page toggles | yes (new without env var) | no, panel entry | yes (+ speed boost) | no (was: page shown, server refused) | yes | no |
| page bots / 1v1 | no | no | yes | no | yes | no |
| re-send | yes (new without env var) | no | yes (new without env var) | no (was: with env var) | yes (new without env var) | no (was: with env var) |
| env-var HUD | yes | no | yes | no (was: shown, server refused) | yes | no (was: shown, server ignored) |

An old server (no field) gets the `match_mode` table, so a worker round
from an old server still shows the page and the server still refuses.

## Tests

- shared 99 → 101: `debug::tests::access_encodes_as_a_small_tolerant_object`,
  `wire::tests::snapshot_debug_access_is_additive` (golden unchanged, `None`
  omitted, `Some` round-trips after `sandbox`, an old snapshot decodes to
  `None`).
- server 297 → 298 (+3 ignored):
  `debug::tests::joined_players_receive_the_server_access_in_every_snapshot`
  (dev, local practice, release through the memory transport); the worker
  victory-snapshot test in `match_allocation` asserts `Some(all false)` with
  `match_mode` `"practice"`; the prejoin status reply asserts the field is
  absent.
- client lib 567 → 573 (547 → 553 without `qa`): access fallback and Combat Test
  (`debug::tests::access_prefers_the_server_value_and_falls_back_to_the_match_mode`),
  the reset (`debug::tests::toggles_reset_when_access_drops`: dev keeps,
  practice → release and practice → worker reset), the re-send without the
  env var, the HUD following access, the page per access, the page reset,
  and the Combat Test entry.

Gate: fmt, both clippy runs, workspace tests, harness (22 unit + 24 black-box) and Python script tests (124, 1 skipped) green locally.
