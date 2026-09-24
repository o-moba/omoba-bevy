# Combat Test / Dev Sandbox

Combat Test is an unranked, opt-in local laboratory that uses the actual Omoba server, hero kits, movement/collision, items, projectiles and damage receipts. It skips account onboarding, matchmaking, draft and loading acknowledgements. It does not change ordinary practice or release matches.

## Launch

From the repository, with the normal Rust and native graphics prerequisites installed:

```sh
python3 scripts/combat_test.py
```

The launcher builds the locked client/server sources, starts a separate loopback server at `127.0.0.1:4040`, and opens the hero picker. Choose a class and appearance, then **Enter Combat Test**. Close the game or press Ctrl+C in the launching terminal to stop only this launcher's processes. An occupied port is an error, not permission to kill another service; use `--bind 127.0.0.1:4041`.

For direct entry with a known hero and built-in scenario:

```sh
python3 scripts/combat_test.py --hero mage --avatar agnes --preset dps
```

To reuse explicit current binaries without recompiling:

```sh
python3 scripts/combat_test.py --hero ranger --preset duel \
  --client-binary target/debug/client --server-binary target/debug/server
```

`--hero` accepts warrior, mage, ranger or cleric. Omit it for the picker. `--avatar` accepts a shipped free roster slug. `--preset` accepts `duel`, `late-game`, `dps`, `animation`, a saved preset name, or a JSON file. Binary paths must refer to matching current builds. The launcher discovers build paths from Cargo's output, including `CARGO_TARGET_DIR` overrides.

For manual startup, the server requires **both** `OMOBA_MATCH_MODE=dev` and `OMOBA_COMBAT_SANDBOX=1`, plus a loopback `SERVER_ADDR`. The client accepts `--combat-test`, optional `--hero`, `--avatar`, and `--sandbox-preset`. Set `GAME_SERVER_ADDR` to the sandbox endpoint. `OMOBA_COMBAT_SANDBOX=1` also enables the client mode when launch arguments are inconvenient. An ordinary server never accepts sandbox commands; the panel reports when connected to the wrong server.

The launcher strips inherited career/database, worker, public service, QA and Ekza overrides. It uses fresh per-session client preferences and explicitly selects the 3D renderer. Logs are printed at startup under `target/combat-test/`. Saved presets use the separate `target/combat-presets/` directory and survive launcher sessions and normal rebuilds; back them up before deleting `target` or running `cargo clean`. `OMOBA_SANDBOX_PRESET_DIR` can select another directory for direct client launches.

## Panel and shortcuts

- **F6:** open/close the Dev Panel. Close it to return to combat input.
- **F7:** pause/resume the authoritative simulation.
- **F9:** advance one 1/60-second simulation frame while paused.
- Click a number to replace it with keyboard input; **Enter** applies, **Esc** cancels. Adjacent -/+ buttons make small adjustments. Scroll within the panel to reach all controls; its close button remains outside the scrolling body.
- Changes display their server acknowledgement or rejection. Lost UDP requests retry using the same identifier, so XP/grants/frame steps are not applied twice.
- Panel input is modal and does not issue world attacks/movement. Teleport closes the panel and asks for a left click on a walkable map point; Esc cancels.

The old God Mode/speed buttons stop controlling the client during Combat Test. They remain available in ordinary explicitly enabled developer UI.

## Heroes, resources, levels and equipment

**Hero** edits your own actor. **Enemy** edits a dedicated opposing bot. Select any shipped class or free appearance in-session. Class changes reset the actor; appearance and stat changes preserve live state unless a reset or changed position is requested.

Controls include God Mode, refill HP/mana, infinite mana, base max HP, movement and attack-speed multipliers, damage multiplier, no cooldowns, reset cooldowns, teleport, actor reset and reset both/duel. Omoba currently models mana, not a separate energy resource. God Mode blocks damage; it does not silently imply infinite mana. Refill and reset can revive an actor at the authoritative test position.

Level is 1–10, individual Q/W/E/R ranks are 1–3, and Unlock All permits all four skills at any level. Early/Mid/Late choose levels 1/5/10, with max ranks at late game. Add/remove XP walks the ordinary progression thresholds in both directions. The ordinary hotbar reads the same authoritative unlocks/ranks as the panel.

Any of the six shipped items can be granted without paying gold or returning to base. Inventory still permits at most six distinct items; clearing it removes their effects. Equipment bonuses stack with sandbox multipliers. Base max HP adds existing level growth (18 per level above 1) and item HP. Lowering the maximum clamps current HP; raising it does not implicitly heal.

Supported bounds: base HP 1–1,000,000; armor/resistance 0–10,000; movement/attack multipliers 0.1–10; damage multiplier 0–100; aggression range 0–100 metres; desired attack distance 0.1–100 metres, capped by real reach when pursuing. Invalid/nonfinite values, invalid ranks/items, and nonwalkable/blocked positions reject the whole change.

## Dummy and damage measurements

In **Dummy**, enable the training target, set HP/armor/resistance, toggle infinite HP, choose stationary/moving behavior, or place it with Teleport. It is a real opposing combat actor, so use normal attacks and skills against it.

In **Damage**, select the target. The meter shows confirmed damage, hits, last hit, DPS and separate source/ability rows. Existing floating damage numbers remain visible in the world. **Reset meter** clears the measurement window and all rows.

- Physical basic/ambient hits use armor; Q/W/E/R projectile hits use resistance. Sandbox mitigation is `incoming damage × 100 / (100 + defense)`; ordinary matches remain unchanged.
- Finite HP records health actually removed, excluding overkill. Infinite dummy HP records the full mitigated hit while preserving health. God Mode generates no damage receipts.
- DPS is confirmed damage divided by simulation seconds since reset, including idle time. Pause freezes the denominator. Source kind and ID separate hero/minion hits, whose numeric IDs can overlap. Measurements accumulate server-side and do not depend on receiving every short-lived visual event.

## Bot and duels

Enable the enemy, choose its hero, progression, loadout and combat settings. **Stand** does nothing; **Flee** moves away when threatened; **Attack** approaches and uses basic attacks; **Fight** also uses available abilities. Aggression range determines engagement; attack distance controls pursuit without granting extra reach. Choose automatic respawn or manually refill/reset after a death. **Cast Q/W/E/R** forces an attempt through ordinary mana, unlock, range and cooldown validation; errors explain rejected attempts.

The `duel` preset starts player versus bot with both at level 10. **Reset both / duel** restores configured positions, resources, progression and loadouts, clears projectiles/minions and measurements, and restores world structures/neutrals/buffs. It keeps the connection and client process alive. Switch classes directly from Hero/Enemy without restarting.

For two real local clients, start a server and then two clients in separate terminals:

```sh
python3 scripts/combat_test.py --server-only
python3 scripts/combat_test.py --connect 127.0.0.1:4040 --hero mage
python3 scripts/combat_test.py --connect 127.0.0.1:4040 --hero ranger
```

Two human seats are assigned opposite sides automatically. Each client controls its own Player configuration; enemy/dummy/environment controls are shared lab state. Start without a bot preset for a pure two-human duel. This developer endpoint is loopback-only and must not be publicly exposed. Normal LAN/iPad play continues using the separate practice server.

## Environment, animation and geometry

**World** enables/disables ordinary minions, spawns a wave, pauses minions, controls simulation speed (`0.1x / 0.25x / 0.5x / 1x / 2x`) and global pause/frame stepping. Disabled minions and their projectiles are removed and no waves spawn. Paused minions stop moving, attacking and spawning; already airborne projectiles continue until global pause.

All combat clocks, movement, cooldowns, AI, regen, respawn and measurements follow simulation time. UI, transport keepalives, snapshot delivery and connection timeouts use real time, so pausing longer than the disconnect timeout remains safe.

**Motion** selects your hero or enemy, lists the loaded graph's real available states and current state, and previews idle/run/walk/attack/skill/hit/death. Preview affects only the model pose. Missing clips are reported with explicit fallbacks, including Attack for absent Hit; preview is not evidence that an authored Hit clip exists. Cube/non-skeletal models cannot provide skeletal animation inspection. Stop Preview returns to combat-driven animation. Repeat replays the same preview or last skill parameters. Pause and frame-step operate on actual clip playback; they do not pause networking.

The overlay shows HP/mana, progression, damage, mitigation, effective movement speed, attack-speed multiplier, cooldowns, motion state, and authoritative/client positions. Geometry colors: red actor hurtbox, gold basic reach, orange projectile volume; Q cyan, W purple, E pink, R green. Cast/basic circles show the kit reach; target-surface allowance is applied by normal combat resolution.

## Presets

Built-ins are `duel` (level-10 hero versus hero), `late-game` (max build and waves), `dps` (infinite target and mana), and `animation` (unlocked skills, no cooldowns, quarter speed). **Presets** also saves and loads a named JSON configuration. Save captures current actor positions as reset baselines. It includes heroes/appearances, levels/XP/ranks/unlocks, resources/stat overrides, equipment, enemy behavior, dummy and world/time controls. Load applies atomically, then resets the scenario after acknowledgement. Malformed/unknown-version/out-of-bounds presets cannot partially change server state. A preset name uses 1–64 letters, digits, hyphens or underscores.

## Adding a command

1. Add a typed command and, if needed, snapshot/config fields in `shared/src/sandbox.rs`.
2. Validate every input and mode/identity boundary in `server/src/sandbox.rs` before mutation. Reuse the ordinary combat path. Do not reset replay counters when resetting cooldowns.
3. Add the panel action in `client/src/sandbox/ui.rs`; queue it through `SandboxClient::submit` so epoch/request IDs, retry and ACK handling stay consistent. Expose the authoritative result in a snapshot or visible acknowledgement.
4. Add behavioral tests under `server/src/sandbox/tests.rs` and relevant client modules, including rejection in ordinary modes, invalid input and duplicate delivery. Extend the native workflow if the command changes layout or animation.

Validation commands: `cargo test --locked -p shared -p server`, `cargo test --locked -p client --lib`, `python3 -m unittest discover -s scripts -p test_combat_test.py`, and `cargo fmt --all -- --check`. Some pre-existing PostgreSQL integration tests require separately provisioned services and remain ignored by default. The opt-in `OMOBA_SANDBOX_QA_OUTPUT` harness uses synthetic UI interactions and a real UDP server with native screenshots; it is explicitly not a claim of physical mouse or iPad input testing.
