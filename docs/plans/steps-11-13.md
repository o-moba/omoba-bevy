# Plan: roadmap steps 11, 12 and 13

Read-only analysis of `main` at 4efbd8b (after PR #33); line numbers drift as slices land. Slices and status are tracked in [REFACTORING.md](../REFACTORING.md). Finding 1 below (debug toggles in worker-allocated rounds) and findings 2-4 (scripts missing Warden) were fixed in a separate PR.

**Step 11 status:** 11-0 (#34) and 11a-11c (#42) are done; 11d is done together with step 15's 15e (notes in [progress/2026-09-24-client-debug-session.md](../progress/2026-09-24-client-debug-session.md)). 11e and 11f are done together (maintainer approval 2026-09-25; notes in [progress/2026-09-25-debug-access.md](../progress/2026-09-25-debug-access.md)), so step 11 is complete. 11e/11f differences from the design: the server sends `debug_access` only to joined players (`Some` also when all false, so a worker round hides the page); the client resets both toggles whenever they are not allowed (not only on the edge out of practice), and the speed boost too; the page is titled "Debug tools" and gains a speed boost line; the `OMOBA_DEBUG_UI` HUD buttons and F2/F3 stay behind the env var but also follow access. 11d differences from the design: the practice page clears `DebugToggles::god_mode` on the edge out of a practice match (not every frame outside one), so a HUD toggle in a dev match survives; `DebugToggles` is initialised by `PlayerPlugin` (local movement reads it) as well as by the debug plugins; the client test count went 561 → 562 for the test that pins the reset.
**Step 13 status:** 13a-13e are done (this PR; notes in [progress/2026-09-25-shared-no-io.md](../progress/2026-09-25-shared-no-io.md)), with the load-time roster validation of report O5 and the working-directory slice of O30; 13f is optional and open. Differences from the design: the working-directory manifest candidates are removed (only the override, the asset root and the embedded copy remain); `CharacterChoice` is listed in `wire_enums.rs` (added on `main` by #47); test counts shared 99 → 92, client lib 567 → 571, passport 18 → 26 (counts on `main` had moved since the plan).

**Step 11 status:** 11-0 (#34) and 11a-11c (#42) are done; 11d is done together with step 15's 15e (notes in [progress/2026-09-24-client-debug-session.md](../progress/2026-09-24-client-debug-session.md)). 11e changes behaviour (the tools page in dev without `OMOBA_DEBUG_UI`, the re-send in practice) and waits for the owner's decision; 11f is optional. 11d differences from the design: the practice page clears `DebugToggles::god_mode` on the edge out of a practice match (not every frame outside one), so a HUD toggle in a dev match survives; `DebugToggles` is initialised by `PlayerPlugin` (local movement reads it) as well as by the debug plugins; the client test count went 561 → 562 for the test that pins the reset.

## Found while planning (bugs outside the slices)

1. **Debug toggles work in rounds that save a durable result (server).** A player can turn on god mode or the speed boost after a worker-allocated practice round has started, and the round still saves as `public-casual-v1`.
   - `handle_set_god_mode` and `handle_set_speed_boost` only check `rules.debug_commands` (`server/src/runtime/handlers/debug.rs:22,56`).
   - `debug_commands` is true for every Practice rules value (`match_rules.rs:202`).
   - A worker with fewer than 10 humans runs `MatchMode::Practice` (`runtime/mod.rs:215-221`).
   - The "no god mode, no speed boost" check runs only once, when the round record is created (`career_runtime.rs:582-589`, eligibility at 536-543). It is not re-checked later.
   - It is reachable from the normal UI: the pause-menu Practice page shows its God mode toggle whenever `match_mode` is `"practice"` (`client/src/practice_sandbox.rs:224-228`), and an allocated practice worker reports exactly that.
   - Practice commands already refuse workers (`bots.rs:644`); the toggles do not.
   - Fix: slice 11-0 below, which can land on its own right away.
2. `scripts/combat_test.py:26`: the `--hero` choices leave out `warden`, although the client accepts it (`client/src/sandbox/mod.rs:30-37`, whose error text also lists only four classes).
3. `scripts/check_combat_balance.py:8,24-25` expects 4 classes and 48 rows, but `server/src/balance_probe.rs:235-247` iterates `HeroClass::ALL` (5 classes, 75 rows). A fresh capture fails the checker.
4. `scripts/verify_beta_match.py:136`: the list of offensive classes leaves out warden, whose E and R abilities deal damage (`shared/src/lib.rs:529-578`).

---

# Step 11: one debug tooling family

## 11.1 Inventory

**A. Development toggles: god mode and speed boost**
- **Wire:** `ClientPacket::SetGodMode{enabled}` and `SetSpeedBoost{enabled}` (`shared/src/protocol/wire.rs:109-114`).
  - Pinned by `GOLDEN_SET_GOD_MODE` (wire.rs:718), the fixtures at 572-573 and `CLIENT_PACKET_TAGS` (18 entries).
  - The harness sends both packets (`harness/src/bot.rs:240-247`), used by `combat_actions.rs:20-21`, `combat_feedback.rs:108,141,144` and `gameplay.rs:52-53,148,163,194,444-445`. **They must stay decodable permanently.**
- **Server dispatch:** `dispatch.rs:319-322`, after the paused-sandbox gate, on the sandbox clock `now`. Both handlers return `Continue`, so the post-command tail runs.
- **Server handlers:**
  - god mode: `handlers/debug.rs:15-48` sets `god_mode`, sets `infinite_resource` unless `sandbox_allowed()`, refills, clears the respawn.
  - speed boost: `handlers/debug.rs:50-73` sets `move_speed_mult = DEBUG_SPEED_MULTIPLIER`.
  - Readers of the flags: `combat_feedback.rs:188`, `sim/mod.rs:91`, `career_runtime.rs:541,587`, and `sandbox.rs:65-72` (echoes them into the `ActorConfig` telemetry).
- **Server gate:** `rules.debug_commands` (Dev and Practice) plus a joined player. There is no worker check (finding 1).
- **Offline:**
  - god mode is a local flag with a refill (`net/offline.rs:136-137,193-204`); damage skips it at 625 and 699.
  - `SetSpeedBoost` falls into `_ => {}` (offline.rs:316). It only appears to work because client prediction moves faster and the offline simulation accepts any finite transform (206-223).
- **Client senders:**
  - `god_mode.rs:135-158` (F2/F3), 160-201 (HUD buttons), 207-227 (re-sends every 0.5 s).
  - All of these are gated by `debug_controls_enabled` (65-67): `OMOBA_DEBUG_UI` (`debug_console.rs:9-22`) and not in Combat Test.
  - The practice page's God mode toggle (`practice_sandbox.rs:292-305`) writes into `DebugToggleState` so the periodic re-send does not undo it.
- **Client state, currently in three places:**
  - `DebugToggleState` (`god_mode.rs:48-51`)
  - `PracticeSandboxState.god_mode` (`practice_sandbox.rs:40`)
  - `DebugSpeedBoost` (`player.rs:34`), which local movement reads at 1205 and 1441, and which is mirrored into `NetworkState.speed_boost_active` (`net/apply.rs:117-124`, `net/mod.rs:120`, `components.rs:45`).
- **Client network commands:** `NetworkCommand::SetGodMode` and `SetSpeedBoost` (`net/commands.rs:56-61`), encoded at 391-398 and 407-414 behind `join_confirmed`.

**B. Practice commands**
- **Wire:** `ClientPacket::Practice{command}` (wire.rs:115-118).
  - `PracticeCommand` (`shared/src/practice.rs:17-29`): Roster, ClearBots, SpawnDummy, StartDuel{level,gold}.
  - It is internally tagged on `kind`; an unknown kind fails the whole packet. `duel()` clamps values (31-38).
  - Constants `DUEL_*` and `MAX_DUMMIES` at 7-14.
- **Server dispatch:** `dispatch.rs:246`, before the pause gate, on the wall clock, returning `Break`. It goes through `handlers/debug.rs:76-87` to `bots.rs:637-743` `handle_practice_command`.
- **Server helpers in `bots.rs`:**
  - `DUMMY_MAX_HP` and `DUMMY_DISTANCE` (205-209)
  - `BotControllers.sandbox`, the flag for "roster replaced" (214-218)
  - `auto_rank_skills` (343-362) and `auto_shop` (364-386); ordinary lane bots also use both (838-841)
  - `configure_duelist` (389-408), `place_dummy` (411-417), `opposite_team` (419-424), `dummy_anchor` (745-765)
- **Server gate:** `rules.fills_with_bots && worker().is_none()`, joined human only; SpawnDummy and StartDuel also require `Running` (bots.rs:644-658, 672, 711).
- **Offline:** `offline.rs:403-507` reimplements all four commands with different details:
  - Roster uses a ring of bots instead of lane bots.
  - The dummy anchor ignores structures (425-446).
  - `DUMMY_HP` and `DUMMY_DISTANCE` are copied (27-28).
  - The duelist uses its own `ranks_for_level` (33-48) and `shop_with` (50-60).
- **Client sender:** the pause-menu page `practice_sandbox.rs:306-329`, shown when `is_practice` matches the string literals `"practice" | "offline_practice"` (224-228). Offline sets that value at offline.rs:752.

**C. Combat Test**
- **Wire:**
  - `ClientPacket::Sandbox{request}` (wire.rs:40-42).
  - `SandboxRequest` carries epoch, match id and request id, plus 12 `SandboxCommand`s (`shared/src/sandbox.rs:161-205`).
  - Acknowledgements and telemetry come back in `Snapshot.sandbox` (wire.rs:470-471; `SandboxSnapshot` at sandbox.rs:246-255).
  - Presets are `SandboxConfig` with `deny_unknown_fields` and `PRESET_VERSION`.
- **Server:**
  - Dispatch: `dispatch.rs:247-249` (`Break`), the paused gate at 250-263, and the virtual clock at 236.
  - Handlers: `handlers/debug.rs:90-102`, then `sandbox.rs:288-293` `sandbox_allowed`, 338-380 (sequencing and acks), 382-544 (the commands), 712+ `simulate_sandbox`.
- **Gate:** enabled only at startup (`runtime/mod.rs:242-254`): `OMOBA_COMBAT_SANDBOX=1`, Dev mode, a loopback bind, not public, no worker.
- **Offline:** none. Offline snapshots send `sandbox: None` (offline.rs:769), and Sandbox packets are dropped by the `_ => {}` arm.
- **Client:**
  - `sandbox/mod.rs:60-79` (launch options), 101-190 (the queue), 236-322 (resend every 300 ms, 8 s timeout).
  - `sandbox/ui.rs` is a 1071-line panel built on the old `frontend::widgets`; also `presets.rs` and `ui/qa.rs`.
  - Launched by `scripts/combat_test.py`.

**D. `debug_console.rs`** is an on-screen log gated by `OMOBA_DEBUG_UI`. It does not send commands.

**Which modes accept what (server authority)**

| Command | release | dev | dev + Combat Test | practice (local) | practice (worker) | offline |
|---|---|---|---|---|---|---|
| god mode / speed boost | ignored | yes | yes (god mode without infinite resource) | yes | **yes (bug)** | god mode yes; speed boost silently ignored |
| Practice commands | ignored | ignored | ignored | yes | ignored | yes (ring roster) |
| Sandbox commands | ignored | ignored | yes | ignored | ignored | ignored |

## 11.2 Design

**Worth unifying**
- **One in-process command family covering the three existing packets:**
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum DebugCommand { GodMode(bool), SpeedBoost(bool), Practice(PracticeCommand) }
  ```
  - It lives in new `shared/src/debug.rs` and **does not derive Serialize or Deserialize**.
  - `to_packet()` and `from_packet(&ClientPacket) -> Option<DebugCommand>` map it onto the existing packets.
  - `shared::practice` is kept, re-exported from `shared::debug`.
  - New constants: `DUMMY_MAX_HP` and `DUMMY_DISTANCE` (removing the copies in `bots.rs:205-209` and `offline.rs:27-28`), and `OFFLINE_PRACTICE_MODE = "offline_practice"`.
- **One permission predicate per side:**
  - Shared, pure: `DebugAccess { toggles, practice }` with `for_match_mode(&str)` and `allows(DebugCommand)`.
  - Server: `ServerRuntime::debug_access()` is `{ toggles: rules.debug_commands && worker().is_none(), practice: rules.fills_with_bots && worker().is_none() }`.
  - A parity test checks, for each mode, that `DebugAccess::for_match_mode(rules.mode_id())` equals `debug_access()`.
- **One server entry point:** `ServerRuntime::handle_debug(addr, DebugCommand, now) -> ControlFlow<()>` in new `server/src/debug/{mod.rs, toggles.rs, practice.rs}`.
  - `practice.rs` takes the practice orchestration out of `bots.rs` (205-209, 389-424, 634-765). Bot AI and the roster primitives (`spawn_bot`, `remove_all_bots`, `remove_bot`) stay in `bots.rs` as `pub(crate)`.
- **One offline entry point:** `Simulation::debug(&mut self, DebugCommand)`, with the speed boost handled explicitly as a no-op and a comment explaining why.
  - I don't recommend a shared trait. Server handlers need `now` and `ControlFlow`; offline has its own clock. An exhaustive `match DebugCommand` on both sides gives the same compile-time guarantee.
- **Shared pure helpers for the policies that are implemented twice today:**
  - `shared::progression::skill_upgrade_order(class, level, ranks, points) -> Vec<u8>`: ultimate first, then Q, W, E. Replaces offline `ranks_for_level` and feeds server `auto_rank_skills`.
  - `shared::shop::plan_purchases(class, gold, owned) -> Vec<ItemId>`: replaces offline `shop_with`, drives server `auto_shop` (still calling `handle_purchase` for each item) and harness `choose_shop_item` (`bot_ai.rs:343-351`).
  - Server practice tests and offline tests both assert against these, so both hosts are tested against the same expected outcomes.
- **Client:** new `client/src/debug/` module.
  - `DebugToggles { god_mode, speed_boost }` replaces the three state copies.
  - One re-send system.
  - `NetworkCommand::Debug(DebugCommand)` replaces three variants.
  - Files move: `god_mode.rs` becomes `debug/hud.rs`, `debug_console.rs` becomes `debug/console.rs`, `practice_sandbox.rs` becomes `debug/tools_page.rs`. The `Name`s stay; no script references them.
  - A `DebugPlugins` group, the group step 10e explicitly leaves for step 11 (`docs/plans/client-10-15.md` §10.2).

**Keep separate**
- **The Combat Test protocol** stays its own family. It is acknowledged, sequenced and scoped to an epoch; it applies a whole `SandboxConfig` atomically; its three actors sit at fixed addresses (40001, 40002); it has a virtual clock with pause, time scale and frame step; it collects damage analytics; it runs only in dev on loopback.
  - Offline cannot host it: there are no minions, structures or virtual clock there.
  - `ActorConfig` (full stat override plus `BotBehavior`) and the practice duel (level plus gold turned into a realistic lane bot) are different abstractions.
  - `DummyConfig` (one configurable dummy with infinite HP or movement) and the practice dummy (a 600 HP Warrior that returns to its anchor, at most 4) are also different.
- **Don't impose the acknowledgement protocol on toggles or practice commands.** They are idempotent, and the toggles are re-sent periodically.
- **Don't fold the Combat Test panel into the pause-menu page.** It is a multi-tab editor with text fields, teleport picking and overlays; moving it to the UI kit is item 4 of step 9b (`docs/ui-kit.md:131`).
  - The tools page uses the kit's `toggle_row`, `adjust_row` and `button` (as `practice_sandbox.rs:142-214` already does); the 9b migration of `sandbox/ui.rs` reuses those widgets.
  - Step 11 only adds a "Combat Test panel" entry on the page when `snapshot.sandbox.is_some()`, and hides the toggles there because `ActorConfig` owns them.

**Wire compatibility**
- **No new `ClientPacket` variant.** It is internally tagged and not tolerant of unknown tags: a `{"type":"debug"}` from a new client would be dropped by an old server, which the protocol rules forbid.
- The existing packets stay the wire encoding, so golden strings and `CLIENT_PACKET_TAGS` are unchanged.
- Future practice commands: first add `#[serde(other)] Unsupported` to `PracticeCommand` (serde allows a unit `other` variant in internally tagged enums). After that, new kinds are additive under the protocol rules.
- An optional additive field `Snapshot.debug_access: Option<DebugAccess>` (`serde(default)`, skipped when `None`, so `GOLDEN_SNAPSHOT` is unchanged) lets the server drive the tools page. This removes the client's string matching and the worker-practice mismatch.

## 11.3 Slices

| # | Slice | Size | Crates | Notes |
|---|---|---|---|---|
| 11-0 | Toggles refuse worker-allocated rounds (`debug.rs:22,56` also checks `worker().is_none()`) | S | server | **Behaviour change / integrity fix.** Test uses the worker fixture in `match_allocation.rs` tests (≈249): SetGodMode after round start leaves `modifiers.god_mode` false. Can land now |
| 11a | `shared/src/debug.rs`: `DebugCommand`, `to_packet`/`from_packet`, `DebugAccess`, constants; `#[serde(other)] Unsupported` on `PracticeCommand` | S | shared | Tests: each command's packet encoding equals today's golden and fixture strings; an unknown kind decodes to `Unsupported`. Shared 78 → about 81 |
| 11b | `server/src/debug/` with `handle_debug` and `debug_access` plus the parity test; `bots.rs` loses the practice orchestration (about 180 lines); `handlers/debug.rs` keeps only `handle_sandbox_packet` (rename to `tools.rs`) | M | server | Dispatcher arms keep their exact positions and clocks (see risks). Server 282 → 283 |
| 11c | Shared planners (`skill_upgrade_order`, `plan_purchases`); server `auto_rank_skills`/`auto_shop`, harness `choose_shop_item` and offline duel use them | S/M | shared, server, harness, client (`offline.rs` only) | An equivalence test runs the old and new functions over all classes × levels 1-10 × budgets 0-1000. Needs 12d |
| 11d | `client/src/debug/`: `DebugToggles`, `NetworkCommand::Debug`, `DebugPlugins`; file moves; offline `Simulation::debug`; the `mirror_debug_flags_to_network_state` copy goes away (the local-player snapshot stage reads `DebugToggles` directly) | M | client | After 10c, 10e and 15b2. Client test count unchanged |
| 11e | One tools page driven by `DebugAccess`: toggles in dev, practice and offline; bots section only where practice is allowed; Combat Test entry; re-send toggles whenever toggles are allowed (today only with `OMOBA_DEBUG_UI`) | S/M | client | **Behaviour change:** dev gets the page without the env var, and practice gets the re-send. Needs the maintainer's OK |
| 11f (opt) | `Snapshot.debug_access` sent by the server | S | shared, server, client | **Wire-visible, additive.** Old clients ignore it; a new client with an old server falls back to `for_match_mode` |

## 11.4 Risks
- **Dispatcher order and clocks.** Today Practice runs before the pause gate on the wall clock and returns `Break`; the toggles run after it on the sandbox clock and return `Continue` (the post-command tail runs `fill_practice_bots`, `tick_prematch`, `register_career_participant`). `handle_debug` must be called from the same arms, with the same `now`, returning the same `ControlFlow` per variant.
- **God mode means different things in three places:** the dev toggle also sets infinite resource except in Combat Test; offline god mode is a local flag. Keep each.
- **Merging client state:** the practice page resets god mode when you leave a practice match (`practice_sandbox.rs:255-258`); the HUD never resets. The single resource must reset when access drops, and a test should pin that.
- **Merge conflicts:** `player.rs` (10c moves `DebugSpeedBoost`), `lib.rs`/`plugins.rs` (10e), `net/apply.rs` (15b, hazard 6), `net/commands.rs`.
- **Behaviour change in 11-0:** in allocated practice rounds, the practice page's god-mode line will claim success while the server refuses, until 11e or 11f.

---

# Step 12: data-driven hero and item catalogs

## 12.1 Consumers
- **The model** (`shared/src/lib.rs`):
  - `BasicAttackDefinition` and `basic_attack_for_class` (125-163)
  - `AbilityDefinition` and `ability()` (175-221)
  - `HeroClass` with `display_name`, `tagline`, `primary_role`, `abilities`, `ability` (225-306)
  - the five `*_ABILITIES` constants (323-578) and `ability_for_class_slot` (580-584)
- **Hero balance** (`hero_balance.rs`): `DEFAULT_MAX_HP = base_hp(Warrior)` (16, a const), `base_hp` (46-54), and per-class growth caps in `basic_damage_multiplier` and `attack_rate_multiplier` (61-84).
- **Shop** (`shop.rs`): `ItemId` (11-40; `from_id` searches `ITEMS`), `ITEMS: [ItemDefinition; INVENTORY_CAPACITY]` (80-143), `item` (145-150), `recommended_items` (153-197).
- **Other shared:** `combat.rs:31-37` `ProjectileStyle::for_class`, and the Warden passive in `jungle.rs:27-47`.
- **Accessor call counts:** server 35 calls across `bots`, `shop`, `sandbox`, `sim/cast`, `basic_attack`, `vision`, `balance*` and tests; client 30 across `combat`, `targeting`, `offline`, `mobile_controls`, `team`, `sandbox/ui`, `skill_icons`; harness 9 in `bot_ai.rs`.
- **Direct `ITEMS` uses:**
  - client `shop.rs:428`, `sandbox/ui.rs:410`, `sandbox/presets.rs:19`
  - server `shop.rs:263` (test), `sandbox/tests.rs:89,277`
- **Direct kit constant use:** harness `combat_actions.rs:122` (`WARRIOR_ABILITIES[0]`).
- **Ability text fields:** about 15 sites read `.id`, `.name` or `.description` as `&'static str` (`combat.rs:880,1112-1169`, `mobile_controls.rs:1122,1374`, `skill_icons.rs:47`, `team.rs:895,973`, `sim/cast.rs:35`, `balance.rs:198`).
- **Per-class tables outside shared:**
  - Keep: server `bots.rs:33-39` `BOT_COMPOSITION` and 185-203 `bot_avatar` (server policy).
  - Keep as presentation: client `skill_icons.rs:5-26` (atlas order, already tested against the kits), `minimap.rs:535-538`, `shop.rs:110-119` `item_code`, `client/assets/config/combat_visuals.json` `classes`.
  - Duplicate that drifts: `targeting.rs:1008-1014`, a test that copies the attack-rate caps.
- **Python:** `combat_test.py:26`, `capture_combat.py:24` (`STYLES`), `verify_beta_match.py:19-21` (`STARTING_GOLD`, `ITEM_COSTS`) and 136, `check_combat_balance.py:8`.

## 12.2 Design
- **Files:** `shared/assets/catalog/heroes.json` and `items.json`, following the existing pattern of `shared/assets/reactions.json` with `social.rs:358-361`. Use JSON, not RON: `serde_json` is already a dependency and Python reads JSON natively.
  - `heroes.json`: `{schema_version, classes:[...]}` in `HeroClass::ALL` order. Each class has `id`, `display_name`, `tagline`, `role`, `base_hp`, `growth{basic_damage_cap, attack_rate_cap}`, `basic_attack{range, damage, cooldown_secs}`, `projectile_style`, `abilities[4]` and `recommended_items[]`.
  - Each ability has `id`, `name`, `description`, `targeting`, `mana_cost`, `cooldown_secs`, `cast_range`, and one of `projectile_damage`, `self_heal` or `self_mana_restore`. `max_rank` is always `MAX_ABILITY_RANK`.
  - `items.json`: `{schema_version, items:[{id, name, description, cost, bonuses}]}` in `ItemId::ALL` order. `bonuses` is a partial `ItemBonuses`; its existing `serde(default)` fills the rest.
- **Loader:** new `shared/src/catalog.rs` with two `static ... : LazyLock<...>` tables built from `include_str!`.
  - Private `Raw*` structs with `deny_unknown_fields`, validated at load time; the load panics with a message on bad data.
  - Converted into the **unchanged public types**. `&'static str` fields are kept by calling `String::leak` once per process (about 100 strings), so `AbilityDefinition` and `ItemDefinition` stay `Copy` and none of the ~15 field-access sites change.
  - Borrowing from the `include_str!` text would also work, but fails on any JSON escape sequence.
  - `catalog::ensure_loaded()` is called from server `runtime::run` and client `main`, so a bad file fails at startup instead of on the first tick.
  - No runtime override (unlike the avatar manifest): both sides must agree, so the data is embedded only.
- **Accessors** keep their names and signatures but lose `const`: `basic_attack_for_class`, the `HeroClass` text/role/kit methods, `hero_balance::{base_hp, basic_damage_multiplier, attack_rate_multiplier}`, `shop::{item, recommended_items}`, `ProjectileStyle::for_class`. New `shop::items() -> &'static [ItemDefinition]` replaces `ITEMS`.
  - `ItemId::from_id` becomes driven by the enum (new `ItemId::ALL`), so wire decoding never touches the catalog.
- **Stays in code:** `HeroClass`, `HeroClass::ALL`, `HeroClass::id`, `ItemId`, `SkillSlot`, `MAX_ABILITY_RANK`, `SLOT_UNLOCK_LEVELS`, rank scaling (586-611), and the global balance values in `hero_balance` (`MAX_LEVEL`, `LEVEL_XP_THRESHOLDS`, `PLAYER_SPEED`, …).
  - Rule of thumb: what differs per class or item is data; what is uniform is code.
  - `STARTING_GOLD`, `INVENTORY_CAPACITY` and `SHOP_RADIUS` stay code. `SHOP_RADIUS` is read in a server const (`balance.rs:21`); `LEVEL_XP_THRESHOLDS[0]` is read in a const in `wire.rs:32`.
  - The Warden passive in `jungle.rs` is behaviour, so it stays code.
- **`const` usages that block the move:** only `DEFAULT_MAX_HP` (`hero_balance.rs:16`), re-exported as `MAX_HP` by client `combat.rs:25`, server `balance.rs:18` and harness `gameplay.rs:28`. None of those use it in a const context.
  - Decision: make it the literal `220.0` and add a test that it equals `base_hp(Warrior)`.
  - `ITEMS` and `*_ABILITIES` are only used in runtime code, so they become accessors.
- **Item count vs inventory capacity: decouple them.** Today `[ItemDefinition; INVENTORY_CAPACITY]` means a seventh item forces capacity 7.
  - After: capacity is a rule; catalog length is `ItemId::ALL.len()`; each class's recommended list covers every item exactly once.
  - Bots, offline and harness already stop at capacity (`bots.rs:374`, `offline.rs:54`, `bot_ai.rs:344`).
  - The three "all items" sites switch to `.take(INVENTORY_CAPACITY)`: `presets.rs:19` (and its test at 132-136), server `shop.rs:263`, `sandbox/tests.rs:89`.
  - The claim in the `practice.rs:10-11` comment ("the cap buys the full inventory") becomes a test.
- **Python: yes, read the same JSON** from `ROOT/'shared/assets/catalog'` through a small `scripts/catalog.py`:
  - `combat_test.py` `--hero` choices (adds warden — a behaviour change)
  - `verify_beta_match.py` item costs, and its offensive-slot list (becomes "slot has `projectile_damage`")
  - `capture_combat.py` `STYLES`
  - **Not** `check_combat_balance.py`: its baseline is a frozen artefact, so it should take its class list from the baseline's own rows (fixes finding 3).

**Validation tests** (in `catalog.rs`, plus the existing ones that stay):
1. Classes and items match their enums one-to-one and in order; ids round-trip through `from_id` and serde.
2. Every class has a full kit (existing test `lib.rs:955`): 4 abilities, ids unique across classes, exactly one effect each, targeting consistent with the effect, `max_rank == MAX_ABILITY_RANK`, non-empty text.
3. Five classes fill five distinct roles (existing `lib.rs:995`).
4. Basic attack range is shorter than Q range (existing `shop.rs:312`).
5. Item costs are above 0; multipliers are at least 1; flat bonuses are at least 0.
6. Every item is recommended exactly once per class (existing `shop.rs:276`), and unknown ids are rejected at parse.
7. Starter budget: the first recommended item costs at most `STARTING_GOLD`, and `plan_purchases(STARTING_GOLD + DUEL_MAX_GOLD)` fills `INVENTORY_CAPACITY` for every class.
8. `DEFAULT_MAX_HP == base_hp(Warrior)`.
9. Migration only (12b): every catalog field equals the old Rust tables bit for bit (`f32::to_bits`). Deleted with the tables in 12c.

## 12.3 Slices

| # | Slice | Size | Notes |
|---|---|---|---|
| 12a | API prep, no data move: `ItemId::ALL`, `from_id` driven by the enum, `shop::items()` with the 6 `ITEMS` sites switched, harness `combat_actions.rs:122` switched to the accessor, `DEFAULT_MAX_HP` as a literal plus test, tables made `pub(crate)` | S | shared, 3 client lines, 3 server test lines, 1 harness line; all tests unchanged |
| 12b | JSON files, `catalog.rs`, validation, bit-exact equality test; nothing reads the catalog yet | M | shared only; about +6 shared tests |
| 12c | Switch the accessors, drop the `const fn`s and the Rust tables and the equality test, add `ensure_loaded` in both binaries; update the docs ("Adding content" in ARCHITECTURE.md, `docs/balance-tuning.md`) | M | Neutrality is proven by the byte pins in `server/src/tests/player_view.rs` and the golden JSON tests |
| 12d | Decouple item count from inventory capacity (above) | S | No behaviour change while there are 6 items and 6 slots |
| 12e | Python reads the catalog, plus Python tests | S | Adding warden to `combat_test.py` is a behaviour change |
| 12f (opt) | Client cross-checks: `combat_visuals.json` classes cover the catalog; `targeting.rs:1008` reads its caps from `hero_balance` | S | |

## 12.4 Risks
- **Float precision:** JSON → f64 → f32 could in principle differ from a Rust f32 literal. The bit-exact test in 12b catches it.
- **Hot path:** `ability_for_class_slot` runs every server tick. `LazyLock` costs one atomic load; `ensure_loaded` moves parsing out of the first tick.
- **Public API changes** (loss of `const fn`, `*_ABILITIES`, `ITEMS`): the only consumers are inside the workspace.
- **Adding a class still needs code:** the enum variant plus client art (skill atlas, `combat_visuals.json`, minimap letter, audio). "Adding content" in ARCHITECTURE.md must say so.

---

# Step 13: shared model free of I/O

## 13.1 Inventory and classification (`shared/src`)

| Site | What it is | Class | Goes to |
|---|---|---|---|
| `lib.rs:36-65` `client_asset_root` | env `OMOBA_ASSET_DIR`, `current_exe`, `is_dir`, `env!(CARGO_MANIFEST_DIR)`, Android cfg | environment / filesystem | `omoba_passport::assets` (the client and `passport-import` call it) |
| `lib.rs:654` `include_str!` of `client/assets/avatars/manifest.json` | embedded client manifest | moves with its loader | passport |
| `lib.rs:661-701` `manifest_candidates`, `avatar_roster` | env `OMOBA_AVATAR_MANIFEST`, 4 disk paths, `fs::read_to_string`, `eprintln`; arena-sync appends to the file at runtime | data loaded at runtime | `omoba_passport::avatars` |
| `lib.rs:705-762` `STORE_AVATARS`, `register_store_avatar`, `store_avatars`, `avatar_definition`, `normalize_avatar_slug` | process-global mutable registry, entries leaked | runtime registry | passport (then optionally owned by the server runtime) |
| `lib.rs:620-651` `AvatarDefinition` (`passport: Option<ProtectedAvatar>`), `AvatarManifest` | manifest schema, not on the wire | moves with the loader | passport |
| `lib.rs:768-780` `SPRITE_CHARACTER_IDS`, default id | frozen ids used by wire normalization | model | stays |
| `lib.rs:782-927` sprite presentation schema, `include_str!` of the client sprite manifest (859), roster, render fallback | client presentation | presentation | client (`sprite_roster.rs`) |
| `lib.rs:888-895` `normalize_sprite_character_id` | server join/prematch normalization (`session.rs:199`, `prematch.rs:314`) | model | stays, rewritten over the id list (identical, because `lib.rs:1178` asserts the manifest ids equal the list) |
| `wire.rs:27`, `prematch.rs:3` `EkzaCharacter` as `CharacterChoice` | SDK type on the wire | layering | shared-owned enum with identical serde |
| `social.rs:9,221,309-324` `validate_avatar_id`, `ProtectedAvatar`, `ConsumedTicket` in `Entitlements::grant_verified_avatar` (only shared's own tests call it, 522-585) | SDK types in the model | layering | move the grant to passport |
| `map.rs:15`, `navigation/mod.rs:80-83`, `social.rs:360` embedded `shared/assets/*` | model data owned by shared | fine | stays; the step 12 catalogs join these |
| `transport.rs:201-247` `Instant::now` | tests only | fine | stays |

**Call sites that change:**
- **Avatars in the server:** `passport_admission.rs:193,262-305` registers from spawned threads (179-188, 231-243); `dispatch.rs:170`, `session.rs:198`, `prematch.rs:303`, `bots.rs:199-201`, `sandbox.rs:242`; tests at `practice_tests.rs:134`, `release_tests.rs:52`, `tests/cast.rs:118`, `passport_admission.rs:611-816`.
- **Avatars in passport:** `store.rs:103-139,327-341` (the client catalogue registrar), `store/fixture_tests.rs:164,200`, `tests/*_e2e.rs`, `bin/passport-import.rs:5`.
- **Avatars in the client (about 40 sites):** `passport.rs:205-479`, `ekza_account.rs:244-433`, `frontend/{collection,card,draft,loading}.rs`, `match_hud.rs:287`, `model_scale.rs:226,573,672`, `net/offline.rs:64,367`, `sandbox/{mod,presets,ui}.rs`, `team.rs:334,1324,1536,1690,1837`, `visual_qa.rs`, `world.rs:136-162`, `lib.rs:120`, `animation_qa.rs`.
- **Avatars in the harness:** `framed_snapshots.rs:30`, `udp_datagrams.rs:83,214`.
- **Sprites:** `sprite.rs` (about 100 references), `team.rs:124,267-268,487,751,1114`, `career.rs:1143`, `minimap.rs:495,515`, `social.rs:1617`.

## 13.2 Where things go
- **Passport as the adapter crate**, rather than a new crate:
  - It already owns the Ekza avatar boundary and already does filesystem, env (`PASSPORT_URL_ENV`) and HTTP.
  - Client and server both depend on it (`client/Cargo.toml:16`, server `Cargo.toml`).
  - `store.rs:341` is already the client-side registrar.
  - The alternative is a small `omoba-assets` crate (shared, serde_json, SDK without default features), useful only if you want HTTP-free consumers to link it. I'd skip it: the harness can read the manifest itself as a `serde_json::Value`, which also keeps it black-box.
- **Keep the names** (`avatar_roster`, `avatar_definition`, `normalize_avatar_slug`, `register_store_avatar`, `store_avatars`) so the change is a path rewrite. Shared cannot keep re-export shims, because passport depends on shared.
- **Env reads move to the binaries:** `RosterSource { manifest_override, asset_root }::from_env()` is called in server `runtime::run` (near `runtime/mod.rs:240`) and in client `main` (`lib.rs:114-121`).
  - The loader is a pure function over a path list and a reader, falling back to lazy initialisation for tests.
  - Log one startup line: roster size and source.
- **Registry ownership:**
  - Phase 1 moves the global into passport unchanged, because the admission threads register off the main thread.
  - Optional phase 2 gives the server its own `ServerRuntime.avatars`: `CompletedAdmission` carries the definition and registers it in `completed()` on the main thread, and the normalization sites take `&AvatarRegistry`. The client keeps the global (passport's store runtime thread, many free functions).
- **The SDK in shared:** with `default-features = false` it pulls only serde and serde_json (I checked its manifest: the `http` and `bevy` features are off), so it causes no I/O. The issue is layering. After 13b, shared uses it only for `CharacterChoice` and the social entitlements.
  - 13d gives shared its own `CharacterChoice` (same `snake_case` serde, no `other` variant, `slug`, `as_str` and `ALL`), pinned by `GOLDEN_JOIN` and `GOLDEN_SNAPSHOT` (`"character":"ipfs"`).
  - The orphan rule forbids an `EkzaCharacter` conversion impl in the client, so the client gets a helper function used at `world.rs:43-58` and `model_scale.rs:167`.
  - 13e moves `grant_verified_avatar` to passport, copies the small `validate_avatar_id` check, and removes the dependency from `shared/Cargo.toml:10`.
  - Removing it beats an optional feature: every crate that needs SDK types (client, server, passport) already depends on the SDK directly, and career-store, account-api and the harness stop compiling it.

## 13.3 Slices

| # | Slice | Size | Test counts |
|---|---|---|---|
| 13a | Sprite presentation moves to the client; shared keeps the ids and normalization | M | Tests at `lib.rs:1200` and `1236` move and `1178` splits: shared −2, client +3 |
| 13b | Asset root, avatar roster and registry move to `omoba_passport::{assets, avatars}`; about 60 mechanical call-site changes; harness reads the manifest JSON itself | M (wide, mechanical) | Tests at `lib.rs:1067, 1086, 1098, 1111, 1314`: shared −5, passport 21 → 26 |
| 13c | `RosterSource` read at the binary boundary, startup log line, pure loader tests with no env mutation | S | +2 passport |
| 13d | Shared-owned `CharacterChoice`; client conversion helper | S | Goldens unchanged |
| 13e | Remove the SDK from shared; update the crate map in ARCHITECTURE.md ("serde, serde_json") | S | The social SDK test (`social.rs:522`) moves to passport |
| 13f (opt) | Registry owned by the server runtime | M | Isolates the server tests |

## 13.4 Risks
- **13b is one wide PR** (no shim is possible). Keep it a pure path rewrite.
- **Join admission is security-relevant:** `dispatch.rs:147-178` and `passport_admission.rs` must behave identically. Their existing tests pin that.
- **Shared global state in tests:** passport's fixture tests register into the same global registry. Use unique slugs.
- **Embedded fallback:** it moves to `passport/src/avatars.rs` (same file bytes). The dev fallback through `CARGO_MANIFEST_DIR` resolves to the same `client/assets`.
- **Android:** the `client_asset_root` Android branch moves with the function; passport is already an unconditional client dependency.
- **Reference test counts in `REFACTORING.md`** change: shared 78 → about 71.
- **13a and 10d both touch `sprite.rs`.**

---

# Recommended order

10a (in progress) → 10b → 10c → 10d → 10e → 10f → 10g → **12a–12e** → **11-0** (or immediately, as a standalone fix) → **11a → 11b → 11c** → 15a → 15b1 → 15b2 → 15c → 15d → 15e → **11d → 11e (→ 11f)** → **13a → 13b → 13c → 13d → 13e (→ 13f)** → 9b.

Why this order:
- **12 before 11.** Step 12 touches only shared plus a few lines elsewhere, so it collides with nothing in steps 10 or 15. 11c's planners are written against `items()` and the decoupled capacity from 12d. Both steps edit `shop.rs` and `bots.rs`, so running them one after the other avoids rebase pain.
- **11's shared and server parts before 15.** The tracker's own reason applies: server structure is fresh. They have no client conflicts.
- **11's client parts after 10c, 10e and 15b2.** 11d removes the `mirror_debug_flags_to_network_state` copy (hazard 6 in §15.4 of the client plan) once the local-player snapshot stage can read `DebugToggles` directly. `DebugSpeedBoost` leaves `player/` (10c). `DebugPlugins` is the group 10e leaves for step 11.
- **13 last.** It is the widest mechanical diff, across client files that steps 10 and 15 are still moving (`team.rs`, `world.rs`, `frontend/*`, `sprite.rs`). 13a must follow 10d.
- **Compared with the tracker** ("10, 15, 11, 12, 13"): same end state. The differences are that 12 moves ahead, and 11 splits into a server/shared half before 15 and a client half after it.

### Critical Files for Implementation
- /home/user/omoba-bevy/shared/src/lib.rs
- /home/user/omoba-bevy/shared/src/shop.rs
- /home/user/omoba-bevy/server/src/runtime/handlers/debug.rs
- /home/user/omoba-bevy/server/src/bots.rs
- /home/user/omoba-bevy/client/src/net/offline.rs