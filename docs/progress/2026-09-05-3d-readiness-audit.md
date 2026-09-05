# 3D playtest readiness audit — 2026-09-05

## Decision and scope

**The project is a substantial multiplayer prototype, but is not ready for unattended first external playtests.** Controlled developer sessions are useful now. The next milestone should be a repeatable, understandable 8–12 minute 3D match with a clean second round, rather than additional avatars, modes or map content.

This assessment reviews the accumulated working tree preserved as `52aa773` (98 files, 11,725 additions and 708 deletions; workspace version 0.17.0, last prior commit 6c98665/version 0.10.0). Three independent source reviews covered server/architecture, client/UI and gameplay. Packaging follow-up is 0.17.1. No gameplay, network or UI behavior is repaired by this audit. One harness fixture is repaired to remove a dependency on local-only avatars.

Evidence is under `.agent/tasks/REPO-AUDIT-2026-09-05/`. Audit completion means the assessment and preservation were verified; it does **not** certify the game or unfinished earlier tasks as PASS. Source line references below describe the preserved gameplay code. Numerical pacing targets are proposals, not measured results. Source citations are repository-relative `file:line` references; paired lines separated by an en dash denote the inspected range.

## What already exists

- An authoritative Rust server, Bevy client, shared ability definitions, and a real-process UDP harness.
- Release queue formation, balanced teams and countdown, development instant-start, lane bots and a solo 5v5 launch flow.
- Three lanes, waves, towers/base destruction, death/respawn, XP/levels/ranks, four classes, camps, two bosses and team buffs.
- 3D models, environment decoration, scale normalization, ground platforms, animated minions, minimap, targeting, approach-to-cast and progression/victory UI.
- Optional 2D presentation with substantial independent work. Its missing Orchard Comet Centaur sheets remain an explicit failing asset gate; completion of 2D is not required for a focused 3D test.

The architecture can support the next milestone. A full ECS rewrite, engine upgrade, new networking library, account platform, item shop, ranked queue or larger roster is not justified before testing this loop.

## Packaging findings repaired in this session

1. A fresh worktree without `.cargo/config.toml` failed `cargo build --workspace --locked`: Cargo.lock described the SDK as a local path package, despite the manifests declaring a git dependency. Restoring the existing SDK git source/revision changes one lockfile source line, without upgrading dependencies.
2. `client/src/net.rs` loads `minions/slime-green.glb` and `minions/slime-blue.glb`, but the entire minions directory was ignored. The two existing models and their CC0 provenance manifest are now included.
3. The tracked avatar manifest listed 28 entries, including 12 runtime-synced `osa-*` avatars whose files are intentionally ignored. A clean checkout therefore had broken selections. The shipped manifest now contains the 16 existing offline avatars; local downloaded files were preserved. Arena sync can still extend the local roster when explicitly used.
4. The full 5v5 UDP fixture hardcoded one of those local-only avatars. After removing the invalid roster references, the test saw complete snapshots but never crossed 8 KiB because avatar IDs became null. It now selects the longest valid roster avatar and asserts that it round-trips, while retaining the original byte-size/topology requirements. The focused regression passed with an 8193-byte real snapshot.

A native Models3d startup also exposed two remaining asset-loading issues: the global sprite preload still requests the missing Orchard sheets (`client/src/sprite.rs:126–130,154–187`), and 3D startup calls the SDK built-in model catalog even for a shipped avatar (`client/src/world.rs:181–187`), producing an invalid downloaded-GLB warning from IPFS. The selected offline avatar and both minion sets nevertheless loaded. Before tester distribution, make nonselected render assets lazy and ensure the approved roster starts without remote model fetches. This audit does not claim a completely offline/error-free startup.

The patch version records delivery fixes, not additional gameplay features. Local SDK overrides can rewrite Cargo.lock again, so release checks should run from a clean checkout with `--locked`.

## Blocking work before unattended tests

Priority P1 below means required before trustworthy external testing. P2 means a later improvement or a risk to measure. No finding justifies a broad architectural rewrite.

| ID | Priority / player impact | Current evidence | Smallest next task and acceptance check |
| --- | --- | --- | --- |
| G1 | P1 — reconnect restores identity but resets the hero; repeated Join heals/revives/changes loadout | `server/src/main.rs:1062–1101` reclaims then invokes the fresh Join initializer; `server/src/session.rs:211–238` resets position, HP/mana, XP, level, ranks and cooldowns | Separate fresh admission, duplicate Join and reclaim. Wound/level/upgrade a hero, reconnect through the real packet handler, and assert the complete state is preserved; duplicate Join must be idempotent. |
| G2 | P1 — the second round has inconsistent stats and timing | `server/src/session.rs:343–366` retains level/threshold/points/ranks while resetting XP and HP/mana caps, keeps ordinary camp state and enters Running unconditionally; `server/src/main.rs:956` versus `server/src/world.rs:229` gives different first-wave timing | One explicit round-reset contract for players, camps, waves, objectives, buffs and roster. Complete a progressed round and assert every field in round two, including release formation. |
| G3 | P1 — balance tests can be invalidated by normal debug commands | Any joined client reaches SetGodMode/SetSpeedBoost in `server/src/main.rs:1139–1168`; speed multiplier is in `server/src/balance.rs:68` | Reject these commands server-side outside an explicit dev configuration; prove rejection in release and acceptance in dev. Hiding buttons is insufficient. |
| N1 | P1 — a lost Join packet can leave a connected player unable to spawn | Client marks Join committed before a single UDP send (`client/src/net.rs:1199–1217`); prejoin snapshots keep transport healthy. Reconnect also sends once (`:2395–2413`); full-roster rejection is only a server log (`server/src/main.rs:1084–1089`) | Acknowledge/reject Join and retry it idempotently with visible state. Drop the first Join and prove eventual admission or a useful explicit rejection. |
| N2 | P1 for remote/Wi-Fi testing — stale snapshots can roll state backward | Full snapshot has no tick/match identity (`server/src/main.rs:402–417`); the client accepts last arrival (`client/src/net.rs:1284–1323`) | Shared protocol version, match ID and monotonic snapshot tick; drop older ticks. Test reordered, delayed and duplicate datagrams across a rematch. |
| N3 | P1 — separate tester groups can inherit an old match or violate capacity | Running ignores empty-roster formation (`server/src/main.rs:689`); timeout removal continues simulation (`:1281–1307`); already-joined reclaims bypass new-player capacity (`:1075–1081`) | Define reserved disconnect seats and an empty-roster grace/reset policy. Test leave-all → new group and timeout → replacement → reclaim. |
| U1 | P1 — right-click can make normal 3D movement appear broken | `client/src/camera.rs:105–119` toggles follow on right-click/Alt; free camera rotates on all mouse motion (`:245`) and handles Q/W/E for flight (`:256–272`) while combat also handles these keys (`client/src/combat.rs:883–890`); `client/src/player.rs:531,615` disables ground movement while unlocked | One player camera contract, explicit debug flight, rotation only while held, reliable Space recenter. Repeated right-clicks must never accidentally disable movement or cast while moving the camera. |
| U2 | P1 — settings/help can send gameplay commands underneath | `PauseMenuState` is local (`client/src/pause_menu.rs:68,492–511`); pointer gates check Button interaction rather than modal context (`client/src/player.rs:553`, `client/src/combat.rs:652`); casts/camera/minimap lack a shared gate | Shared Gameplay/Modal/Debug input context. With each overlay open, background/text clicks and QWER/U/Y generate no world commands. Online simulation may continue. |
| U3 | P1 — players cannot understand why an ability did nothing | Rejection/approach feedback goes to DebugConsole (`client/src/combat.rs:759–849,1021`), disabled and discarding lines by default (`client/src/debug_console.rs:28–45`) | Visible action feedback and per-slot lock/mana/cooldown/range states with debug UI disabled. Each rejection must be understandable without logs. |
| U4 | P1 candidate, needs runtime layout check — join controls may be clipped | Both avatar grids occupy flex layout; inactive uses Visibility::Hidden (`client/src/team.rs:209,276–339`) and centered root has no scrolling. Packaging reduces 28 to 16 avatars, but the hidden two-row sprite grid still occupies layout | Use Display::None for inactive content; bounded responsive/scrollable active grid. All choices and team buttons reachable at 1280×720 and 1366×768 without resizing. |

## Make the existing 3D combat feel good

These changes have higher expected value than more scenery or characters; enjoyment remains a playtest hypothesis.

1. **Connect existing animations.** Shipped avatar GLBs already contain attack/cast/death along with idle/walk. Hero graphs and state only use idle/walk (`client/src/player.rs:270–295,476–492`); replicated cosmetic action sequences already exist (`client/src/net.rs:1534`). Start with one approved avatar: local and remote attack/cast/death one-shots, death hold, respawn and locomotion transitions without replay on duplicate snapshots. Bosses also only choose idle/walk (`client/src/bosses.rs:330–333`).
2. **Add a small 3D feedback vocabulary.** The existing cast/hit/heal/death effects return early in Models3d (`client/src/presentation2d.rs:710–712`), and projectiles are generic geometry (`client/src/net.rs:1726–1743`). Source/assets contain no gameplay audio. Add clear cast release, impact, healing, death and a few recognizable sounds, with bounded lifetimes. Use existing facilities before adding dependencies.
3. **Make allegiance obvious.** All overhead 3D HP bars use the same red material (`client/src/combat.rs:304–310,1139`); cosmetics are independent of class/team. Add local/friendly/enemy rings or outlines and a non-color cue. Validate mixed teams using the same avatar at all supported zooms.
4. **Consolidate combat UI.** Ability names are 9.5px (`client/src/combat.rs:490`), readiness/descriptions live in a separate text HUD (`client/src/match_hud.rs:388,446`), and target text exposes IDs (`:435`). Prioritize a clear HP/mana/XP bar, readable QWER slots, cooldown overlays, target name/class/HP and one next-objective prompt. Hide implementation IDs from normal play.
5. **Make every visible control honest.** Renderer switch buttons currently log restart/environment instructions without changing/persisting mode (`client/src/team.rs:707–715`). Implement an observable next-start choice or show noninteractive status. Ground clicks intersect y=0 even over 0.7-high base pads (`client/src/player.rs:633`, `client/src/maps.rs:10`); measure the aiming offset before repairing terrain picking.

## Gameplay design and pacing

- **Protect the lane objective loop.** Mage Q range is 30 (`shared/src/lib.rs:243–251`), versus tower range 20 and base range 24 (`server/src/balance.rs:38–43`). Any living hostile structure accepts damage with no lane prerequisite (`server/src/main.rs:1724–1738,2022–2033`). The rules permit safe base sniping. Choose one clear siege rule, such as base protection until a lane tower falls, and communicate defensive ranges. Do not silently tune numbers without testing the intended wave-supported siege.
- **Tune progression for the actual roster.** Minion XP is split globally among joined teammates (`server/src/main.rs:2362–2393`), while jungle rewards go to the killer (`:2122–2131`). Nine enemy minions per wave ×32 XP ÷5 heroes is about 57.6 XP per hero per wave. Level 6 requires 900 cumulative XP, approximately 15.6 complete wave equivalents without jungle, with 60-second wave intervals. Integer rounding, first-wave timing, travel, kills and jungle change actual timing. This is a calculation, not a measured 15.6-minute match. Measure first fight, levels 2/4/6 and match end in 5v5; 1v1 progression is not representative.
- **Two distinct archetypes are enough for the first design test.** Damage abilities share a homing projectile path (`server/src/main.rs:1788–1818`); effects are projectile damage, self-heal and self-mana restore (`shared/src/lib.rs:57–77`). Cleric cannot heal allies. Polish one bruiser and one ranged kit with an identifiable decision each before expanding abilities. A dodgeable attack or ally support can follow observed feedback.
- **Treat bots as scenario support.** Current bots push lanes and use Q; no retreat, W/E/R, rank spending or boss coordination (`harness/src/bot_ai.rs:233–295`, `harness/src/bin/bots.rs:250–254`). First add avoidance of unsupported tower dives, self-sustain and rank spending. These bots exercise traffic/core combat but do not validate coordinated class balance.
- **Close the reward loop simply.** Gold currently has no spending path; player death lacks killer/assist reward attribution (`server/src/main.rs:2007–2019`). A small results screen and useful metrics are a better first investment than a full shop. Either explain/defer unused gold or give it one bounded purpose.

## Architecture and code priorities

Keep the current workspace boundaries and authoritative simulation. Address ownership and contracts where concrete defects occur:

1. Extract a focused match/session module and one canonical fresh-round state initializer while fixing G1/G2/N3. Avoid a broad rewrite of the 5,206-line server entrypoint.
2. Move wire types and shared map geometry into `shared` incrementally. Packet/entity shapes currently repeat in `client/src/net.rs:352`, `server/src/main.rs:47` and `harness/src/protocol.rs:107`. Add compatibility/order tests before changing replication.
3. Document one source of gameplay truth. ServerRuntime HashMaps (`server/src/main.rs:902–924`) are copied into ECS for selected subsystems (`:1435–1556`). Migrate only one subsystem at a time when it removes duplicated state; do not convert everything just to satisfy an ECS style preference.
4. Measure transport before expanding remote tests. Full-world JSON snapshots can exceed 8 KiB in populated 5v5 (`harness/tests/udp_datagrams.rs:31–89`), are cloned/serialized per recipient every 50 ms (`server/src/main.rs:1370–1406`), and have only a 65,507-byte ceiling. This removes old receive truncation but does not solve IP fragmentation or platform send limits. The current host successfully sent/received 9216 bytes and rejected 9217 with EMSGSIZE in a separate loopback boundary probe; the initial failing 5v5 fixture had no server-send errors. Record snapshot bytes, send failures, tick duration, loss and jitter. Compact/MTU-conscious framing is a separate design task.
5. Fix movement tolerance under packet bursts after session blockers: `speed × elapsed + 0.10` is granted per request (`server/src/session.rs:264–288`). Use a cumulative time budget and verify equal travel at different packet rates.
6. Add lightweight build revision/match ID, snapshot age, frame-time/tick-time and disconnect reason diagnostics. A log/CSV is enough for the first sessions.
7. Triage the existing dependency alerts before broader distribution. GitHub reported 16 alerts on the default branch during this push (6 high, 6 moderate, 4 low). These counts are remote-reported and were not individually validated for reachability/exploitability in this audit; no dependency upgrades were made.

The real UDP harness is a strength, but passing it does not prove all lifecycle invariants. Reclaim tests stop before the resetting Join handler (`server/src/main.rs:3490–3526`); rematch tests omit player progression (`:4818–4872`). Always build the current server before integration tests because `harness/src/server.rs:173–207` accepts a prebuilt binary. The older manual matrix only checks Running/base HP after rematch (`scripts/verify_task_12_qa_matrix_live_udp.py:526–530`).

## Ordered delivery plan

| Batch | Outcome | Relative scope / dependencies |
| --- | --- | --- |
| 1 | Reproducible build/assets, clean Join/reconnect/rematch, release debug rejection | Packaging repaired here; lifecycle remains the first engineering task. Several focused changes plus real UDP regressions. |
| 2 | Reliable 3D input and first screen: modal gating, camera recovery, reachable join controls, visible cast feedback | Focused client work; can run alongside batch 1 once input/state boundaries are agreed. |
| 3 | Readable combat: existing one-shots, team cues, compact hotbar, minimal 3D VFX/audio | Start with two archetypes and one approved avatar each; no roster expansion. |
| 4 | Full short match: siege rule, progression pacing, useful bots, results/metrics | Depends on 1–3; balance choices follow recorded sessions. |
| 5 | Repeatable tester build and network/performance gate | Native supported-OS smoke, two humans + 8 bots, loss/reorder tests and soak. Broader remote rollout only after transport evidence. |

These are separate task-sized batches, not an estimate that can be promised from source inspection alone. Freeze acceptance criteria for each before implementation. A coherent small slice is more valuable than partially completing every item at once.

## First test format and go/no-go

First run a 1v1 rule/UI smoke, then **two humans plus eight bots in the existing release 5v5 flow**. Guide players toward the mid-lane fight on the existing map; no map redesign is needed. Use a small curated avatar/class selection in the test brief. Other lanes/camps/bosses can stay in the build without becoming required onboarding material.

Required gate before unattended invitation:

- Fresh checkout or packaged build launches on each declared supported OS without sibling SDK checkout or missing required assets; define one tester launch path and record build SHA.
- All join controls and HUD readable at 720p; four of five new testers can move, attack and identify the next objective within two minutes using in-game guidance.
- No right-click camera traps or gameplay commands through modal overlays; each cast rejection has visible feedback.
- Dropped first Join recovers; reconnect preserves gameplay state; duplicate Join cannot heal; release debug commands are rejected.
- Every test completes victory and a consistent second round. No inherited camp/player/buff state or underfilled release start unless explicitly chosen as policy.
- Five consecutive full-roster sessions plus a 30-minute 3D soak have no crashes, snapshot stalls, ghost accumulation or permanent input failure. Record client frame times, memory trend, server tick time and packet size/errors on named hardware; choose a supported hardware/FPS budget before declaring performance PASS.
- Measure proposed pacing targets: first fight ≤60 s, level 2 ≤2 min, ultimate around 5–7 min, typical finish 8–12 min. Revise these after observation; none is currently claimed to pass.
- Capture winner/duration, level timings, deaths, damage/objectives, disconnects, confusion points, OS/GPU and build revision. Existing bug-report template can remain the starting point.

The existing 10–20-minute manual script makes victory/rematch optional. For this gate they are mandatory. Old release-PASS notes and checked feature inventories must not substitute for current evidence.

## Verification results

- `cargo build --workspace --locked`: initially failed on the missing SDK lockfile source; passed after the narrow packaging repair, using the git SDK at `6e4123aa284cd6faecc44037b2bad25494e29f10` and workspace 0.17.1.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo test --workspace --locked -- --test-threads=1`: initially failed at `harness/tests/udp_datagrams.rs:75`. Diagnosis found complete 5v5 snapshots up to 8113 bytes and no send errors; the test's rejected local-only avatar shortened payloads below the required threshold. Earlier groups passed 109 client tests, 9 harness unit tests and 13 live harness tests; separate server/shared/skills passed 56/11/5. After the fixture repair, `cargo test -p harness --test udp_datagrams --locked -- --nocapture` passed with 8193 bytes. The final full-workspace rerun passed **204 tests, 0 failed** (`raw/test-workspace-final.log`).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `python3 scripts/verify_task_02_multiplayer_session_flow.py`: passed all 6 existing scenarios. Its duplicate-Join scenario explicitly expects last-Join-wins; it is not proof of the idempotent in-match contract recommended here.
- Shipped avatar validator: initially failed on 28 entries and 12 missing runtime-only models; passed for the final 16-avatar manifest. Boss validator and final minion validator passed (missing minion thumbnails are warnings; those models are not player selections).
- Sprite validator: 15 negative self-tests pass, but the real manifest fails because both Orchard sheets are absent. The 2D readability validator fails for the same missing hero. These remain product failures.
- World2D validator self-tests: 6 pass. Full manifest validation fails in the clean worktree because it requires an ignored task-local `provenance.json`; validation is not self-contained in a fresh checkout.
- Live UDP contract probe on the freshly built release 1v1 server completed in 29.5 s. Reclaim kept player ID 2 but restored HP 76→100 and moved the player 97.74 units back to spawn. Duplicate Join moved player 1 back 105.5 units, restored mana 93.37→100 and reset action sequence 2→0. Release SetGodMode restored HP 76→100; SetSpeedBoost increased travel from 11.02 to 23.44 units over similar 1.43 s windows (includes per-packet tolerance; not a speed benchmark). This confirms state reset and release debug acceptance. The probe did not raise levels or execute rematch, so progression reset findings remain source-grounded.
- Native Models3d startup: a 20-second isolated-config run on Apple M4 Pro/Metal stayed alive, received its first snapshot, autojoined the dev server as Megan the Fox, and loaded both minion animation sets. Logs contain the missing optional sprite sheets and invalid SDK model download described above. This is startup evidence, not manual gameplay or visual QA.

 Native UI automation was attempted but could not initialize: Computer Use returned `failed to read /Users/wotori/.codex/config.toml: Permission denied`. No permissions/configuration were changed. Accordingly, no visual layout, manual control feel, screenshot readability or FPS claim is made by this audit.
